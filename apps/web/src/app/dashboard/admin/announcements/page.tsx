'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { Announcement } from '@/types';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { format } from 'date-fns';
import { Info, Plus, Edit, Trash2 } from 'lucide-react';


interface CreateAnnouncementData {
  title: string;
  content?: string;
  type: string;
  expires_at?: string;
}

interface UpdateAnnouncementData {
  title?: string;
  content?: string;
  type?: string;
  is_active?: boolean;
  expires_at?: string;
}

export default function AnnouncementsAdminPage() {
  const queryClient = useQueryClient();
  const [showForm, setShowForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [formData, setFormData] = useState<CreateAnnouncementData>({
    title: '',
    content: '',
    type: 'info',
    expires_at: '',
  });

  const { data: announcements = [], isLoading } = useQuery<Announcement[]>({
    queryKey: ['announcements-admin'],
    queryFn: () => api.get('/announcements/all?limit=100'),
  });

  const createMutation = useMutation({
    mutationFn: (data: CreateAnnouncementData) => api.post('/announcements', data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['announcements-admin'] });
      resetForm();
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateAnnouncementData }) =>
      api.patch(`/announcements/${id}`, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['announcements-admin'] });
      resetForm();
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/announcements/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['announcements-admin'] });
    },
  });

  const toggleActiveMutation = useMutation({
    mutationFn: ({ id, is_active }: { id: string; is_active: boolean }) =>
      api.patch(`/announcements/${id}`, { is_active }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['announcements-admin'] });
    },
  });

  const resetForm = () => {
    setShowForm(false);
    setEditingId(null);
    setFormData({ title: '', content: '', type: 'info', expires_at: '' });
  };

  const handleEdit = (announcement: Announcement) => {
    setEditingId(announcement.id);
    setFormData({
      title: announcement.title,
      content: announcement.content || '',
      type: announcement.type,
      expires_at: announcement.expires_at
        ? announcement.expires_at.slice(0, 16)
        : '',
    });
    setShowForm(true);
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateAnnouncementData = {
      title: formData.title,
      content: formData.content || undefined,
      type: formData.type,
      expires_at: formData.expires_at || undefined,
    };

    if (editingId) {
      updateMutation.mutate({ id: editingId, data });
    } else {
      createMutation.mutate(data);
    }
  };

  const getTypeColor = (type: string) => {
    switch (type) {
      case 'info':
        return 'bg-blue-100 text-blue-700';
      case 'warning':
        return 'bg-amber-100 text-amber-700';
      case 'success':
        return 'bg-emerald-100 text-emerald-700';
      case 'error':
        return 'bg-red-100 text-red-700';
      default:
        return 'bg-gray-100 text-gray-700';
    }
  };

  return (
    <div className="max-w-4xl">
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-2xl font-medium flex items-center gap-2 text-gray-400">
          <Info className="w-6 h-6" />
          Announcements
        </h1>
        {!showForm && (
          <Button
            onClick={() => setShowForm(true)}
            className="h-8 text-sm px-3 bg-violet-600 hover:bg-violet-700"
          >
            <Plus className="w-4 h-4 mr-1.5" />
            New Announcement
          </Button>
        )}
      </div>

      {/* Create/Edit Form */}
      {showForm && (
        <div className="mb-8 p-6 rounded-xl bg-white border border-gray-200">
          <h2 className="text-lg font-medium text-gray-900 mb-4">
            {editingId ? 'Edit Announcement' : 'New Announcement'}
          </h2>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <Label htmlFor="title">Title</Label>
              <Input
                id="title"
                value={formData.title}
                onChange={(e) => setFormData({ ...formData, title: e.target.value })}
                placeholder="Announcement title"
                required
                className="mt-1"
              />
            </div>

            <div>
              <Label htmlFor="content">Content (optional)</Label>
              <textarea
                id="content"
                value={formData.content}
                onChange={(e) => setFormData({ ...formData, content: e.target.value })}
                placeholder="Additional details..."
                rows={3}
                className="mt-1 w-full px-3 py-2 rounded-lg border border-gray-300 focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor="type">Type</Label>
                <select
                  id="type"
                  value={formData.type}
                  onChange={(e) => setFormData({ ...formData, type: e.target.value })}
                  className="mt-1 w-full px-3 py-2 rounded-lg border border-gray-300 focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                >
                  <option value="info">Info</option>
                  <option value="warning">Warning</option>
                  <option value="success">Success</option>
                  <option value="error">Error</option>
                </select>
              </div>

              <div>
                <Label htmlFor="expires_at">Expires At (optional)</Label>
                <Input
                  id="expires_at"
                  type="datetime-local"
                  value={formData.expires_at}
                  onChange={(e) => setFormData({ ...formData, expires_at: e.target.value })}
                  className="mt-1"
                />
              </div>
            </div>

            <div className="flex gap-3 pt-2">
              <Button
                type="submit"
                disabled={createMutation.isPending || updateMutation.isPending}
                className="bg-violet-600 hover:bg-violet-700"
              >
                {(createMutation.isPending || updateMutation.isPending) ? (
                  <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
                ) : editingId ? (
                  'Update'
                ) : (
                  'Create'
                )}
              </Button>
              <Button type="button" variant="outline" onClick={resetForm}>
                Cancel
              </Button>
            </div>
          </form>
        </div>
      )}

      {/* Announcements List */}
      {isLoading ? (
        <div className="flex items-center justify-center py-12">
          <div className="w-8 h-8 border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin" />
        </div>
      ) : announcements.length === 0 ? (
        <div className="text-center py-12 px-4 rounded-xl border-2 border-dashed border-gray-200 bg-gray-50/50">
          <Info className="w-12 h-12 mx-auto text-gray-300 mb-4" />
          <p className="text-gray-500 mb-4">No announcements yet</p>
          <Button
            onClick={() => setShowForm(true)}
            className="bg-violet-600 hover:bg-violet-700"
          >
            Create First Announcement
          </Button>
        </div>
      ) : (
        <div className="space-y-3">
          {announcements.map((announcement) => (
            <div
              key={announcement.id}
              className={`p-4 rounded-xl border ${
                announcement.is_active
                  ? 'bg-white border-gray-200'
                  : 'bg-gray-50 border-gray-100 opacity-60'
              }`}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className={`text-xs font-medium px-2 py-0.5 rounded-full ${getTypeColor(announcement.type)}`}>
                      {announcement.type}
                    </span>
                    {!announcement.is_active && (
                      <span className="text-xs font-medium px-2 py-0.5 rounded-full bg-gray-100 text-gray-600">
                        Inactive
                      </span>
                    )}
                    {announcement.expires_at && new Date(announcement.expires_at) < new Date() && (
                      <span className="text-xs font-medium px-2 py-0.5 rounded-full bg-red-100 text-red-600">
                        Expired
                      </span>
                    )}
                  </div>
                  <h3 className="font-medium text-gray-900 truncate">
                    {announcement.title}
                  </h3>
                  {announcement.content && (
                    <p className="text-sm text-gray-500 mt-1 line-clamp-2">
                      {announcement.content}
                    </p>
                  )}
                  <div className="flex items-center gap-4 mt-2 text-xs text-gray-400">
                    <span>
                      Created: {format(new Date(announcement.created_at), 'MMM d, yyyy HH:mm')}
                    </span>
                    {announcement.expires_at && (
                      <span>
                        Expires: {format(new Date(announcement.expires_at), 'MMM d, yyyy HH:mm')}
                      </span>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => toggleActiveMutation.mutate({
                      id: announcement.id,
                      is_active: !announcement.is_active,
                    })}
                    disabled={toggleActiveMutation.isPending}
                    className={`relative w-10 h-5 rounded-full transition-colors ${
                      announcement.is_active ? 'bg-emerald-500' : 'bg-gray-300'
                    }`}
                    title={announcement.is_active ? 'Deactivate' : 'Activate'}
                  >
                    <span
                      className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                        announcement.is_active ? 'translate-x-5' : 'translate-x-0'
                      }`}
                    />
                  </button>
                  <button
                    onClick={() => handleEdit(announcement)}
                    className="p-2 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded-lg transition-colors"
                    title="Edit"
                  >
                    <Edit className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => {
                      if (confirm('Are you sure you want to delete this announcement?')) {
                        deleteMutation.mutate(announcement.id);
                      }
                    }}
                    disabled={deleteMutation.isPending}
                    className="p-2 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded-lg transition-colors"
                    title="Delete"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
